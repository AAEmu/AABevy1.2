//! Original base cooldown button retains one explicit sample beyond its duration.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "native_cooldown_caption.rs"]
#[allow(dead_code)]
mod original;
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native cooldown snapshot"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            original::install(ui, true);
            ui.exec(
                "@native_snapshot",
                r#"
                panel:SetExtent(560,390)
                local title=panel:CreateChildWidget('label','snapshotTitle',0,true)
                title:SetExtent(540,24);title:AddAnchor('TOP',panel,0,265)
                title:SetText('One SetCoolDown(1200,1500), no provider');title.style:SetFontSize(16)
                snapshot=CreateCoolDownActionButton('snapshotButton',panel,'generic')
                snapshot:SetExtent(ICON_SIZE.DEFAULT,ICON_SIZE.DEFAULT)
                snapshot:AddAnchor('TOP',panel,0,310)
                snapshot:SetItemIcon(4045,2);snapshot:Show(true)
                snapshot:SetCoolDown(1200,1500)
            "#,
            )
            .unwrap();
        }))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
