//! Original base cooldown buttons retain raw native samples and reject invalid arguments.
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
                panel:SetExtent(640,480)
                local title=panel:CreateChildWidget('label','snapshotTitle',0,true)
                title:SetExtent(540,24);title:AddAnchor('TOP',panel,0,265)
                title:SetText('One SetCoolDown(1200,1500), no provider');title.style:SetFontSize(16)
                snapshot=CreateCoolDownActionButton('snapshotButton',panel,'generic')
                snapshot:SetExtent(ICON_SIZE.DEFAULT,ICON_SIZE.DEFAULT)
                snapshot:AddAnchor('TOP',panel,0,310)
                snapshot:SetItemIcon(4045,2);snapshot:Show(true)
                SetViewOfUpdateCooldown(snapshot)
                snapshot:SetCoolDown(1200,1500)
                local rawTitle=panel:CreateChildWidget('label','rawSnapshotTitle',0,true)
                rawTitle:SetExtent(620,24);rawTitle:AddAnchor('TOP',panel,0,365)
                rawTitle:SetText('Over total     Zero total     Rejected string     Negative remain')
                rawTitle.style:SetFontSize(16)
                for i=1,4 do
                    local b=CreateCoolDownActionButton('rawSnapshot'..i,panel,'generic')
                    b:SetExtent(42,42);b:AddAnchor('TOP',panel,(i-2.5)*140,410)
                    SetViewOfUpdateCooldown(b)
                    b:SetItemIcon(4045,2);b:SetCoolDown(8000,12000);b:Show(true)
                    if i==1 then b:SetCoolDown(8000,4000)
                    elseif i==2 then b:SetCoolDown(1000,0)
                    elseif i==3 then b:SetCoolDown('1000',2000)
                    else b:SetCoolDown(-1,12000) end
                end
            "#,
            )
            .unwrap();
        }))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
