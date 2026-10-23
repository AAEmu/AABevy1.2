// Read-only Ghidra Dec.java output; x2game SHA256
// 851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e.
// Function signatures/types are decompiler inference. native-source.txt retains
// the original instructions; user_music_catalog_oracle.py executes the loaders.
// Reproduce with existing proj/x2game, -process x2game_unpacked.dll -readOnly
// -noanalysis -postScript Dec.java <output> 396f6390 39730ee0 39730620 3972c390.
// ===== FUN_396f6390 @ 396f6390 (asked 396f6390) size=62

undefined4 FUN_396f6390(uint param_1)

{
  uint *puVar1;

  for (puVar1 = *(uint **)(PTR_DAT_39acb0e8 + 0x26d8);
      (puVar1 != *(uint **)(PTR_DAT_39acb0e8 + 0x26dc) && (*puVar1 <= param_1)); puVar1 = puVar1 + 3
      ) {
    if (param_1 <= puVar1[1]) {
      return *(undefined4 *)(puVar1[2] + (param_1 - *puVar1) * 4);
    }
  }
  return 0;
}


// ===== FUN_39730ee0 @ 39730ee0 (asked 39730ee0) size=960

undefined4 FUN_39730ee0(undefined4 param_1,int param_2)

{
  int iVar1;
  char cVar2;
  uint uVar3;
  int iVar4;
  undefined4 uVar5;
  char *pcVar6;
  uint *puVar7;
  undefined4 *puVar8;
  undefined4 *puVar9;
  undefined4 local_70 [2];
  undefined1 local_68;
  undefined4 local_64;
  undefined4 local_60;
  undefined4 local_5c;
  undefined1 local_58;
  undefined4 local_54;
  undefined4 local_50;
  undefined1 local_4c;
  undefined1 local_4b;
  undefined1 local_4a;
  undefined4 local_48;
  undefined1 local_44;
  undefined4 local_40;
  undefined4 local_3c;
  undefined4 local_38;
  undefined4 local_34;
  undefined1 local_30;
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
  puStack_c = &LAB_39980018;
  local_10 = ExceptionList;
  uVar3 = DAT_39acb9c0 ^ (uint)&stack0xfffffffc;
  ExceptionList = &local_10;
  local_1c = param_2 + 0x2524;
  cVar2 = FUN_39718780(param_1,"skill_effects");
  if (cVar2 != '\0') {
    param_2 = 0;
    local_8 = 0;
    iVar4 = FUN_3955b3d0(param_1,
                         "SELECT id, always_hit, application_method_id, back, chance, consume_item_count, consume_item_id, consume_source_item, effect_id, end_level, friendly, front, interaction_success_hit, item_set_id, non_friendly, skill_id, source_buff_tag_id, source_nobuff_tag_id, start_level, synergy_text, target_buff_tag_id, target_nobuff_tag_id, target_npc_tag_id, weight FROM skill_effects"
                         ,0xffffffff,&param_2,0,uVar3);
    if (iVar4 == 0) {
      local_14 = 0;
      iVar4 = FUN_3955fb10(param_2);
      if (iVar4 == 100) {
        local_18 = 0;
        do {
          uVar5 = FUN_3953a390(param_2,0);
          pcVar6 = (char *)FUN_3953a480(param_2,1);
          cVar2 = *pcVar6;
          if (((cVar2 == 'T') || (cVar2 == 't')) || (local_70[0]._0_1_ = 0, cVar2 == '1')) {
            local_70[0]._0_1_ = 1;
          }
          local_70[1] = FUN_3953a390(param_2,2);
          pcVar6 = (char *)FUN_3953a480(param_2,3);
          cVar2 = *pcVar6;
          if (((cVar2 == 'T') || (cVar2 == 't')) || (local_68 = 0, cVar2 == '1')) {
            local_68 = 1;
          }
          local_64 = FUN_3953a390(param_2,4);
          local_60 = FUN_3953a390(param_2,5);
          local_5c = FUN_3953a390(param_2,6);
          pcVar6 = (char *)FUN_3953a480(param_2,7);
          cVar2 = *pcVar6;
          if (((cVar2 == 'T') || (cVar2 == 't')) || (local_58 = 0, cVar2 == '1')) {
            local_58 = 1;
          }
          uVar3 = FUN_3953a390(param_2,8);
          for (puVar7 = *(uint **)(iVar1 + 0x24c0);
              (puVar7 != *(uint **)(iVar1 + 0x24c4) && (*puVar7 <= uVar3)); puVar7 = puVar7 + 3) {
            if (uVar3 <= puVar7[1]) {
              local_54 = *(undefined4 *)(puVar7[2] + (uVar3 - *puVar7) * 4);
              goto LAB_39731096;
            }
          }
          local_54 = 0;
LAB_39731096:
          local_50 = FUN_3953a390(param_2,9);
          pcVar6 = (char *)FUN_3953a480(param_2,10);
          cVar2 = *pcVar6;
          if (((cVar2 == 'T') || (cVar2 == 't')) || (local_4c = 0, cVar2 == '1')) {
            local_4c = 1;
          }
          pcVar6 = (char *)FUN_3953a480(param_2,0xb);
          cVar2 = *pcVar6;
          if (((cVar2 == 'T') || (cVar2 == 't')) || (local_4b = 0, cVar2 == '1')) {
            local_4b = 1;
          }
          pcVar6 = (char *)FUN_3953a480(param_2,0xc);
          cVar2 = *pcVar6;
          if (((cVar2 == 'T') || (cVar2 == 't')) || (local_4a = 0, cVar2 == '1')) {
            local_4a = 1;
          }
          local_48 = FUN_3953a390(param_2,0xd);
          pcVar6 = (char *)FUN_3953a480(param_2,0xe);
          cVar2 = *pcVar6;
          if (((cVar2 == 'T') || (cVar2 == 't')) || (local_44 = 0, cVar2 == '1')) {
            local_44 = 1;
          }
          local_40 = FUN_3953a390(param_2,0xf);
          local_3c = FUN_3953a390(param_2,0x10);
          local_38 = FUN_3953a390(param_2,0x11);
          local_34 = FUN_3953a390(param_2,0x12);
          pcVar6 = (char *)FUN_3953a480(param_2,0x13);
          cVar2 = *pcVar6;
          if (((cVar2 == 'T') || (cVar2 == 't')) || (local_30 = 0, cVar2 == '1')) {
            local_30 = 1;
          }
          local_2c = FUN_3953a390(param_2,0x14);
          local_28 = FUN_3953a390(param_2,0x15);
          local_24 = FUN_3953a390(param_2,0x16);
          local_20 = FUN_3953a390(param_2,0x17);
          *(undefined4 *)(*(int *)(local_1c + 0x60) + local_14 * 4) = uVar5;
          puVar9 = (undefined4 *)(*(int *)(local_1c + 100) + local_18);
          local_18 = local_18 + 0x54;
          local_14 = local_14 + 1;
          puVar8 = local_70;
          for (iVar4 = 0x15; iVar4 != 0; iVar4 = iVar4 + -1) {
            *puVar9 = *puVar8;
            puVar8 = puVar8 + 1;
            puVar9 = puVar9 + 1;
          }
          *(int *)(iVar1 + 0x10ac8) = *(int *)(iVar1 + 0x10ac8) + 1;
          iVar4 = FUN_3955fb10(param_2);
        } while (iVar4 == 100);
      }
      if (iVar4 == 0x65) {
        FUN_39725790(local_14);
        if (param_2 != 0) {
          FUN_395519b0(param_2);
        }
        ExceptionList = local_10;
        return 1;
      }
      uVar5 = FUN_395338f0(param_1);
      FUN_396dcb10("failed: sqlite3_step: function=LoadSkillEffectDescs errcode=%d errmsg=%s",iVar4,
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
            "SELECT id, always_hit, application_method_id, back, chance, consume_item_count, consume_item_id, consume_source_item, effect_id, end_level, friendly, front, interaction_success_hit, item_set_id, non_friendly, skill_id, source_buff_tag_id, source_nobuff_tag_id, start_level, synergy_text, target_buff_tag_id, target_nobuff_tag_id, target_npc_tag_id, weight FROM skill_effects"
            ,iVar4,uVar5);
      if (param_2 != 0) {
        FUN_395519b0(param_2);
      }
    }
  }
  ExceptionList = local_10;
  return 0;
}


// ===== FUN_39730620 @ 39730620 (asked 39730620) size=2227

undefined4 FUN_39730620(undefined4 param_1,int param_2)

{
  undefined8 *puVar1;
  int iVar2;
  char cVar3;
  uint uVar4;
  int iVar5;
  undefined4 uVar6;
  char *pcVar7;
  undefined4 uVar8;
  uint *puVar9;
  undefined4 uStack_24;
  int local_14;
  void *local_10;
  undefined1 *puStack_c;
  undefined4 local_8;

  iVar2 = param_2;
  local_8 = 0xffffffff;
  puStack_c = &LAB_3997ffe8;
  local_10 = ExceptionList;
  uVar4 = DAT_39acb9c0 ^ (uint)&stack0xfffffffc;
  ExceptionList = &local_10;
  cVar3 = FUN_39718610(param_1,"effects");
  if (cVar3 != '\0') {
    param_2 = 0;
    local_8 = 0;
    iVar5 = FUN_3955b3d0(param_1,"SELECT id, actual_type, actual_id FROM effects",0xffffffff,
                         &param_2,0,uVar4);
    if (iVar5 == 0) {
      local_14 = 0;
      iVar5 = FUN_3955fb10(param_2);
      while (iVar5 == 100) {
        uVar6 = FUN_3953a390(param_2,0);
        pcVar7 = (char *)FUN_3953a480(param_2,1);
        uVar4 = FUN_3953a390(param_2,2);
        iVar5 = XlCompareString(pcVar7,"BuffEffect");
        if (iVar5 == 0) {
          for (puVar9 = *(uint **)(iVar2 + 0x1550);
              (uStack_24 = 0, puVar9 != *(uint **)(iVar2 + 0x1554) && (*puVar9 <= uVar4));
              puVar9 = puVar9 + 3) {
            if (uVar4 <= puVar9[1]) {
              uVar8 = *(undefined4 *)(puVar9[2] + (uVar4 - *puVar9) * 4);
              goto LAB_39730de0;
            }
          }
LAB_39730746:
          uVar8 = 0;
        }
        else {
          iVar5 = XlCompareString(pcVar7,"DamageEffect");
          if (iVar5 == 0) {
            puVar9 = *(uint **)(iVar2 + 0x13b0);
            uStack_24 = 1;
            if (puVar9 == *(uint **)(iVar2 + 0x13b4)) goto LAB_39730746;
            do {
              if (uVar4 < *puVar9) goto LAB_39730746;
              if (uVar4 <= puVar9[1]) {
                uVar8 = *(undefined4 *)(puVar9[2] + (uVar4 - *puVar9) * 4);
                goto LAB_39730de0;
              }
              puVar9 = puVar9 + 3;
            } while (puVar9 != *(uint **)(iVar2 + 0x13b4));
            uVar8 = 0;
          }
          else {
            iVar5 = XlCompareString(pcVar7,"HealEffect");
            if (iVar5 == 0) {
              puVar9 = *(uint **)(iVar2 + 0x1418);
              uStack_24 = 2;
              if (puVar9 == *(uint **)(iVar2 + 0x141c)) goto LAB_39730746;
              do {
                if (uVar4 < *puVar9) goto LAB_39730746;
                if (uVar4 <= puVar9[1]) {
                  uVar8 = *(undefined4 *)(puVar9[2] + (uVar4 - *puVar9) * 4);
                  goto LAB_39730de0;
                }
                puVar9 = puVar9 + 3;
              } while (puVar9 != *(uint **)(iVar2 + 0x141c));
              uVar8 = 0;
            }
            else {
              iVar5 = XlCompareString(pcVar7,"RestoreManaEffect");
              if (iVar5 == 0) {
                uStack_24 = 3;
                uVar8 = FUN_396e8c80(uVar4);
              }
              else {
                iVar5 = XlCompareString(pcVar7,"AggroEffect");
                if (iVar5 == 0) {
                  uStack_24 = 4;
                  uVar8 = FUN_396e8cc0(uVar4);
                }
                else {
                  iVar5 = XlCompareString(pcVar7,"SpawnEffect");
                  if (iVar5 == 0) {
                    uStack_24 = 5;
                    uVar8 = FUN_396e8d80(uVar4);
                  }
                  else {
                    iVar5 = XlCompareString(pcVar7,"SpawnGimmickEffect");
                    if (iVar5 == 0) {
                      uStack_24 = 6;
                      uVar8 = FUN_396e8dc0(uVar4);
                    }
                    else {
                      cVar3 = FUN_39005b60(pcVar7,"PhysicalExplosionEffect");
                      if (cVar3 == '\0') {
                        cVar3 = FUN_39005b60(pcVar7,"ImpulseEffect");
                        if (cVar3 == '\0') {
                          cVar3 = FUN_39005b60(pcVar7,"TrainCraftEffect");
                          if (cVar3 == '\0') {
                            cVar3 = FUN_39005b60(pcVar7,"AcceptQuestEffect");
                            if (cVar3 == '\0') {
                              cVar3 = FUN_39005b60(pcVar7,"ManaBurnEffect");
                              if (cVar3 == '\0') {
                                cVar3 = FUN_39005b60(pcVar7,"ConversionEffect");
                                if (cVar3 == '\0') {
                                  cVar3 = FUN_39005b60(pcVar7,"InteractionEffect");
                                  if (cVar3 == '\0') {
                                    cVar3 = FUN_39005b60(pcVar7,"CraftEffect");
                                    if (cVar3 == '\0') {
                                      cVar3 = FUN_39005b60(pcVar7,"KillNpcWithoutCorpseEffect");
                                      if (cVar3 == '\0') {
                                        cVar3 = FUN_39005b60(pcVar7,"DispelEffect");
                                        if (cVar3 == '\0') {
                                          cVar3 = FUN_39005b60(pcVar7,"ReportCrimeEffect");
                                          if (cVar3 == '\0') {
                                            cVar3 = FUN_39005b60(pcVar7,"ImprintUccEffect");
                                            if (cVar3 == '\0') {
                                              cVar3 = FUN_39005b60(pcVar7,"NpcControlEffect");
                                              if (cVar3 == '\0') {
                                                cVar3 = FUN_39005b60(pcVar7,"NpcSpawnerSpawnEffect")
                                                ;
                                                if (cVar3 == '\0') {
                                                  cVar3 = FUN_39005b60(pcVar7,
                                                  "NpcSpawnerDespawnEffect");
                                                  if (cVar3 == '\0') {
                                                    cVar3 = FUN_39005b60(pcVar7,"BubbleEffect");
                                                    if (cVar3 == '\0') {
                                                      cVar3 = FUN_39005b60(pcVar7,"SkillController")
                                                      ;
                                                      if (cVar3 == '\0') {
                                                        cVar3 = FUN_39005b60(pcVar7,"SpecialEffect")
                                                        ;
                                                        if (cVar3 == '\0') {
                                                          cVar3 = FUN_39005b60(pcVar7,
                                                  "ScopedFEffect");
                                                  if (cVar3 == '\0') {
                                                    cVar3 = FUN_39005b60(pcVar7,"CleanupUccEffect");
                                                    if (cVar3 == '\0') {
                                                      cVar3 = FUN_39005b60(pcVar7,"OpenPortalEffect"
                                                                          );
                                                      if (cVar3 == '\0') {
                                                        cVar3 = FUN_39005b60(pcVar7,
                                                  "RepairSlaveEffect");
                                                  if (cVar3 == '\0') {
                                                    cVar3 = FUN_39005b60(pcVar7,
                                                  "GainLootPackItemEffect");
                                                  if (cVar3 == '\0') {
                                                    cVar3 = FUN_39005b60(pcVar7,"CinemaEffect");
                                                    if (cVar3 == '\0') {
                                                      cVar3 = FUN_39005b60(pcVar7,"RecoverExpEffect"
                                                                          );
                                                      if (cVar3 == '\0') {
                                                        cVar3 = FUN_39005b60(pcVar7,
                                                  "ResetAoeDiminishingEffect");
                                                  if (cVar3 == '\0') {
                                                    cVar3 = FUN_39005b60(pcVar7,
                                                  "MoveToRezPointEffect");
                                                  if (cVar3 == '\0') {
                                                    cVar3 = FUN_39005b60(pcVar7,
                                                  "PutDownBackpackEffect");
                                                  if (cVar3 == '\0') {
                                                    cVar3 = FUN_39005b60(pcVar7,"SpawnFishEffect");
                                                    if (cVar3 == '\0') {
                                                      cVar3 = FUN_39005b60(pcVar7,
                                                  "FlyingStateChangeEffect");
                                                  if (cVar3 == '\0') {
                                                    cVar3 = FUN_39005b60(pcVar7,"PlayLogEffect");
                                                    if (cVar3 == '\0') {
                                                      cVar3 = FUN_39005b60(pcVar7,
                                                  "AccountAttributeEffect");
                                                  if (cVar3 == '\0') {
                                                    FUN_396dcb10(
                                                  "failed: invalid polymorphic type: %s",pcVar7);
                                                  if (param_2 == 0) {
                                                    ExceptionList = local_10;
                                                    return 0;
                                                  }
                                                  FUN_395519b0(param_2);
                                                  ExceptionList = local_10;
                                                  return 0;
                                                  }
                                                  uStack_24 = 0x26;
                                                  uVar8 = FUN_396e91c0(uVar4);
                                                  }
                                                  else {
                                                    uStack_24 = 0x25;
                                                    uVar8 = FUN_396e8d00(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x24;
                                                    uVar8 = FUN_396e9280(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x23;
                                                    uVar8 = FUN_396e9240(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x22;
                                                    uVar8 = FUN_396e9440(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x21;
                                                    uVar8 = FUN_396e9400(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x20;
                                                    uVar8 = FUN_396e93c0(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x1f;
                                                    uVar8 = FUN_396e9380(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x1e;
                                                    uVar8 = FUN_396e8f80(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x1d;
                                                    uVar8 = FUN_396e9040(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x1c;
                                                    uVar8 = FUN_396e9340(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x1b;
                                                    uVar8 = FUN_396e9300(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x1a;
                                                    uVar8 = FUN_396e92c0(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x19;
                                                    uVar8 = FUN_396e9200(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x18;
                                                    uVar8 = FUN_396e8d40(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x17;
                                                    uVar8 = FUN_396ff9c0(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x16;
                                                    uVar8 = FUN_396e9180(uVar4);
                                                  }
                                                  }
                                                  else {
                                                    uStack_24 = 0x15;
                                                    uVar8 = FUN_396e9500(uVar4);
                                                  }
                                                }
                                                else {
                                                  uStack_24 = 0x14;
                                                  uVar8 = FUN_396e94c0(uVar4);
                                                }
                                              }
                                              else {
                                                uStack_24 = 0x13;
                                                uVar8 = FUN_396e9480(uVar4);
                                              }
                                            }
                                            else {
                                              uStack_24 = 0x12;
                                              uVar8 = FUN_396e9000(uVar4);
                                            }
                                          }
                                          else {
                                            uStack_24 = 0x11;
                                            uVar8 = FUN_396e9140(uVar4);
                                          }
                                        }
                                        else {
                                          uStack_24 = 0x10;
                                          uVar8 = FUN_396e9100(uVar4);
                                        }
                                      }
                                      else {
                                        uStack_24 = 0xf;
                                        uVar8 = FUN_396e90c0(uVar4);
                                      }
                                    }
                                    else {
                                      uStack_24 = 0xe;
                                      uVar8 = FUN_396e9080(uVar4);
                                    }
                                  }
                                  else {
                                    uStack_24 = 0xd;
                                    uVar8 = FUN_396e8fc0(uVar4);
                                  }
                                }
                                else {
                                  uStack_24 = 0xc;
                                  uVar8 = FUN_396e8f00(uVar4);
                                }
                              }
                              else {
                                uStack_24 = 0xb;
                                uVar8 = FUN_396e8ec0(uVar4);
                              }
                            }
                            else {
                              uStack_24 = 10;
                              uVar8 = FUN_396e8f40(uVar4);
                            }
                          }
                          else {
                            uStack_24 = 9;
                            uVar8 = FUN_396e8e80(uVar4);
                          }
                        }
                        else {
                          uStack_24 = 8;
                          uVar8 = FUN_396e8e40(uVar4);
                        }
                      }
                      else {
                        uStack_24 = 7;
                        uVar8 = FUN_396e8e00(uVar4);
                      }
                    }
                  }
                }
              }
            }
          }
        }
LAB_39730de0:
        *(undefined4 *)(*(int *)(iVar2 + 0x251c) + local_14 * 4) = uVar6;
        puVar1 = (undefined8 *)(*(int *)(iVar2 + 0x2520) + local_14 * 0xc);
        *puVar1 = CONCAT44(uStack_24,uVar6);
        *(undefined4 *)(puVar1 + 1) = uVar8;
        *(int *)(iVar2 + 0x10ac8) = *(int *)(iVar2 + 0x10ac8) + 1;
        local_14 = local_14 + 1;
        iVar5 = FUN_3955fb10(param_2);
      }
      if (iVar5 == 0x65) {
        FUN_397255f0(local_14);
        if (param_2 != 0) {
          FUN_395519b0(param_2);
        }
        ExceptionList = local_10;
        return 1;
      }
      uVar6 = FUN_395338f0(param_1);
      FUN_396dcb10("failed: sqlite3_step: function=LoadEffectDescs errcode=%d errmsg=%s",iVar5,uVar6
                  );
      if (param_2 != 0) {
        FUN_395519b0(param_2);
        ExceptionList = local_10;
        return 0;
      }
    }
    else {
      uVar6 = FUN_395338f0(param_1);
      X2Log(4,"failed: sqlite3_prepare_v2: sql=\'%s\' errcode=%d errmsg=%s",
            "SELECT id, actual_type, actual_id FROM effects",iVar5,uVar6);
      if (param_2 != 0) {
        FUN_395519b0(param_2);
      }
    }
  }
  ExceptionList = local_10;
  return 0;
}


// ===== FUN_3972c390 @ 3972c390 (asked 3972c390) size=474

undefined4 FUN_3972c390(undefined4 param_1,int param_2)

{
  char cVar1;
  uint uVar2;
  int iVar3;
  undefined4 uVar4;
  undefined4 uVar5;
  undefined4 uVar6;
  undefined4 uVar7;
  undefined4 uVar8;
  undefined4 uVar9;
  int iVar10;
  int local_18;
  int local_14;
  void *local_10;
  undefined1 *puStack_c;
  undefined4 local_8;

  local_8 = 0xffffffff;
  puStack_c = &LAB_3997f918;
  local_10 = ExceptionList;
  uVar2 = DAT_39acb9c0 ^ (uint)&stack0xfffffffc;
  ExceptionList = &local_10;
  cVar1 = FUN_39715230(param_1,"special_effects");
  if (cVar1 != '\0') {
    local_14 = 0;
    local_8 = 0;
    iVar3 = FUN_3955b3d0(param_1,
                         "SELECT id, special_effect_type_id, value1, value2, value3, value4 FROM special_effects"
                         ,0xffffffff,&local_14,0,uVar2);
    if (iVar3 == 0) {
      iVar10 = 0;
      iVar3 = FUN_3955fb10(local_14);
      if (iVar3 == 100) {
        local_18 = 0;
        do {
          uVar4 = FUN_3953a390(local_14,0);
          uVar5 = FUN_3953a390(local_14,1);
          uVar6 = FUN_3953a390(local_14,2);
          uVar7 = FUN_3953a390(local_14,3);
          uVar8 = FUN_3953a390(local_14,4);
          uVar9 = FUN_3953a390(local_14,5);
          *(undefined4 *)(*(int *)(param_2 + 0x167c) + iVar10 * 4) = uVar4;
          iVar3 = *(int *)(param_2 + 0x1680);
          *(ulonglong *)(iVar3 + local_18) = CONCAT44(uVar6,uVar5);
          *(ulonglong *)(iVar3 + 8 + local_18) = CONCAT44(uVar8,uVar7);
          *(undefined4 *)(iVar3 + 0x10 + local_18) = uVar9;
          *(int *)(param_2 + 0x10ac8) = *(int *)(param_2 + 0x10ac8) + 1;
          local_18 = local_18 + 0x14;
          iVar10 = iVar10 + 1;
          iVar3 = FUN_3955fb10(local_14);
        } while (iVar3 == 100);
      }
      if (iVar3 == 0x65) {
        FUN_39721c10(iVar10);
        if (local_14 != 0) {
          FUN_395519b0(local_14);
        }
        ExceptionList = local_10;
        return 1;
      }
      uVar4 = FUN_395338f0(param_1);
      FUN_396dcb10("failed: sqlite3_step: function=LoadSpecialEffectDescs errcode=%d errmsg=%s",
                   iVar3,uVar4);
      if (local_14 != 0) {
        FUN_395519b0(local_14);
        ExceptionList = local_10;
        return 0;
      }
    }
    else {
      uVar4 = FUN_395338f0(param_1);
      X2Log(4,"failed: sqlite3_prepare_v2: sql=\'%s\' errcode=%d errmsg=%s",
            "SELECT id, special_effect_type_id, value1, value2, value3, value4 FROM special_effects"
            ,iVar3,uVar4);
      if (local_14 != 0) {
        FUN_395519b0(local_14);
      }
    }
  }
  ExceptionList = local_10;
  return 0;
}
