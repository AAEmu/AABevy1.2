use super::*;

#[test]
fn visible_grade_labor_matches_original_x86_oracle() {
    let path = std::path::Path::new(aa_anim::db::DEFAULT_DB);
    if !path.exists() {
        return;
    }
    let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let tables = Tables::load(&conn).unwrap();
    assert_eq!(tables.grades, vec![1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(tables.experts.len(), 31);
    for (base, points, grade, expected) in [
        (100, 10000, 1, 100),
        (100, 19999, 1, 100),
        (100, 20000, 1, 95),
        (10, 20000, 1, 10),
        (0, 20000, 1, 1),
        (100, 30000, 2, 90),
    ] {
        assert_eq!(labor(&tables, 1, base, points, grade), expected);
    }
    assert_eq!(labor(&tables, 0, 100, 100000, 6), 0);
    assert_eq!(tables.recipes.len(), 7010);
    assert_eq!(tables.recipes.values().map(|r| r.materials.len()).max(), Some(8));
    assert!(tables.recipes.values().flat_map(|r| &r.materials).all(|m| m.item != 0));
    assert_eq!(tables.benches[&274].len(), 3, "CraftStart lists also contribute recipes");
    assert_eq!(tables.benches[&568].len(), 71);
    assert_eq!(tables.benches[&2241].len(), 153);
    assert!(tables.craft_skills.contains(&tables.recipes[&5557].skill));
}

#[test]
#[ignore = "isolated original world stage and physical input probe"]
fn original_recipe_book_groups_categories_and_recipe_selection() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = fixture::runtime(1920.0, 1080.0)?;
    rt.lua
        .load(
            r#"
      local groups=X2Craft:EnumActabilityGroupTypes(); assert(#groups==34)
      local categories=X2Craft:GetActabilityCategoryTypesGroupType(1); assert(#categories>0)
      assert(X2Craft:GetCraftBaseInfo(0)==nil and X2Craft:GetCraftMaterialInfo(0)==nil)
      assert(X2Craft:GetCraftProductInfo(0)==nil and X2Craft:GetExecutableCraftCount(0)==nil)
      assert(not X2Craft:IsWorkingCraft() and X2Craft:GetLeftBatchCount()==0)
      assert(X2Ability:GetMyActabilityInfo(1).point==0)
    "#,
        )
        .exec()?;
    fixture::select_first_recipe(&rt)?;
    rt.lua.load("assert(CRAFTING_FRAME.detailPage:IsVisible() and not CRAFTING_FRAME.abilityGroupPage:IsVisible()); assert(#CRAFTING_FRAME.detailPage.categoryList.content:GetViewItemsInfo()>0); assert(not CRAFTING_FRAME.detailPage.buttonFrame:IsVisible())").exec()?;
    aa_ui::lua::flush(&rt.lua);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    drop(bridge);
    drop(rt);
    std::fs::remove_file(path)?;
    Ok(())
}
